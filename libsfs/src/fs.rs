use std::{
    collections::{HashMap, HashSet},
    fs::File as StdFile,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use aes_gcm::{
    aead::{rand_core::RngCore, Aead, OsRng},
    AeadCore, Aes256Gcm, Key, KeyInit,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use rkyv::{Archive, Deserialize, Serialize};
use snafu::{OptionExt, ResultExt};
use snap::raw::{Decoder, Encoder};
use xxhash_rust::xxh3::xxh3_64;

use crate::{
    config::Config,
    error::{Error, FsError, NotFoundError, Result},
    filetime,
};

#[derive(Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize, Clone)]
#[archive(check_bytes)]
pub struct FileRecord {
    #[serde(skip)]
    checksum: u64,
    #[serde(skip)]
    offset: usize,
    #[serde(skip)]
    len: usize,
    #[serde(skip)]
    nonce: [u8; 12],
    #[serde(skip)]
    is_compressed: bool,
    size: usize,
}

#[derive(Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize, Clone)]
#[archive(check_bytes)]
pub struct DirectoryRecord {
    entries: HashMap<String, usize>,
}

#[derive(Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize, Clone)]
#[archive(check_bytes)]
pub struct SymlinkRecord {
    reference_record_id: usize,
    is_file: bool,
}

#[derive(Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize, Clone, Default)]
#[serde(tag = "tag")]
#[archive(check_bytes)]
pub enum RecordInner {
    #[default]
    Empty,
    File(FileRecord),
    Directory(DirectoryRecord),
    Symlink(SymlinkRecord),
}

#[derive(Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize, Clone, Default)]
#[archive(check_bytes)]
pub struct Record {
    pub id: usize,
    pub name: String,
    file_times: filetime::FileTimes,
    inner: RecordInner,
}

impl Record {
    fn as_file(&self) -> Result<&FileRecord> {
        let RecordInner::File(file) = &self.inner else {
            return Err(Error::NotFile {
                name: self.name.clone(),
            });
        };

        Ok(file)
    }

    pub fn as_directory(&self) -> Result<&DirectoryRecord> {
        let RecordInner::Directory(directory) = &self.inner else {
            return Err(Error::NotDirectory {
                name: self.name.clone(),
            });
        };

        Ok(directory)
    }

    pub fn as_directory_mut(&mut self) -> Result<&mut DirectoryRecord> {
        let RecordInner::Directory(directory) = &mut self.inner else {
            return Err(Error::NotDirectory {
                name: self.name.clone(),
            });
        };

        Ok(directory)
    }
}

pub struct RecordTable {
    config: Config,
    cipher: Aes256Gcm,
    compressor: Encoder,
    decompressor: Decoder,
    backend: StdFile,
    working_dir_id: usize,
    meta: Meta,
    crypt: Crypt,
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[archive(check_bytes)]
struct Crypt {
    user_salt: [u8; 16],
    store_key: Vec<u8>,
    store_nonce: [u8; 12],
    meta_nonce: [u8; 12],
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone)]
#[archive(check_bytes)]
pub struct Meta {
    entries: Vec<Record>,
    free_fragments: Vec<(usize, Vec<usize>)>,
    empty_records: Vec<usize>,
    end_offset: usize,
    pinned: HashSet<usize>,
}

impl RecordTable {
    pub fn config(&self) -> Config {
        self.config.clone()
    }

    pub fn pin(&mut self, record_id: usize) {
        self.meta.pinned.insert(record_id);
    }

    pub fn unpin(&mut self, record_id: usize) {
        self.meta.pinned.remove(&record_id);
    }

    pub fn working_dir(&self) -> &Record {
        &self.meta.entries[self.working_dir_id]
    }

    pub fn working_dir_mut(&mut self) -> Result<&mut DirectoryRecord> {
        self.meta.entries[self.working_dir_id].as_directory_mut()
    }

    pub fn set_working_dir_id(&mut self, record_id: usize) {
        self.working_dir_id = record_id;
    }

    pub fn get_dir_entries(&self, id: usize) -> Result<(Record, Vec<Record>)> {
        Ok((
            self.meta.entries[id].clone(),
            self.meta.entries[id]
                .as_directory()?
                .entries
                .values()
                .map(|id| self.meta.entries[*id].clone())
                .collect(),
        ))
    }

    pub fn init(user_key: &str, base: &Path) -> Result<Self> {
        if base.exists() {
            Self::open(user_key, base)
        } else {
            std::fs::create_dir_all(base).context(FsError {
                path: base.display().to_string(),
            })?;

            Self::new(user_key, base)
        }
    }

    fn new(user_key: &str, base: &Path) -> Result<Self> {
        let config = Config::new(base)?;
        let verify_salt = SaltString::generate(&mut OsRng);

        let mut salt = [0; 16];
        OsRng.fill_bytes(&mut salt);

        let mut out = [0; 32];
        Argon2::default().hash_password_into(user_key.as_bytes(), &salt, &mut out)?;

        std::fs::write(
            &config.key,
            Argon2::default()
                .hash_password(user_key.as_bytes(), &verify_salt)?
                .to_string(),
        )?;

        let store_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&out));
        let store_nonce = Aes256Gcm::generate_nonce(OsRng);
        let meta_nonce = Aes256Gcm::generate_nonce(OsRng);

        let store_key = Aes256Gcm::generate_key(OsRng);
        let root_record = Record {
            id: 0,
            name: String::new(),
            file_times: filetime::now(),
            inner: RecordInner::Directory(DirectoryRecord {
                entries: HashMap::default(),
            }),
        };

        Ok(Self {
            cipher: Aes256Gcm::new(&store_key),
            compressor: Encoder::new(),
            decompressor: Decoder::new(),
            backend: StdFile::options()
                .create_new(true)
                .write(true)
                .read(true)
                .open(&config.storage)
                .context(FsError {
                    path: config.storage.to_string_lossy(),
                })?,
            working_dir_id: 0,
            config,
            meta: Meta {
                entries: vec![root_record],
                free_fragments: Vec::new(),
                empty_records: Vec::new(),
                end_offset: 0,
                pinned: HashSet::new(),
            },
            crypt: Crypt {
                store_key: store_cipher.encrypt(&store_nonce, store_key.as_slice())?,
                store_nonce: store_nonce.into(),
                meta_nonce: meta_nonce.into(),
                user_salt: salt,
            },
        })
    }

    fn open(user_key: &str, base: &Path) -> Result<Self> {
        let config = Config::new(base)?;
        let crypt = rkyv::from_bytes::<Crypt>(&std::fs::read(&config.crypt).context(FsError {
            path: config.crypt.to_string_lossy(),
        })?)?;

        let mut out = [0; 32];
        Argon2::default().hash_password_into(user_key.as_bytes(), &crypt.user_salt, &mut out)?;

        let store_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&out));
        let store_key =
            store_cipher.decrypt(&crypt.store_nonce.into(), crypt.store_key.as_slice())?;

        let cipher = Aes256Gcm::new(store_key.as_slice().into());

        let meta_file_data = std::fs::read(&config.meta).context(FsError {
            path: config.meta.to_string_lossy(),
        })?;

        let meta_data = cipher.decrypt(&crypt.meta_nonce.into(), meta_file_data.as_slice())?;

        let meta = rkyv::from_bytes::<Meta>(&meta_data)?;

        Ok(Self {
            cipher,
            compressor: Encoder::new(),
            decompressor: Decoder::new(),
            backend: StdFile::options()
                .write(true)
                .read(true)
                .open(&config.storage)
                .context(FsError {
                    path: config.storage.to_string_lossy(),
                })?,
            working_dir_id: 0,
            config,
            meta,
            crypt,
        })
    }

    pub fn close(&mut self) -> Result<()> {
        let meta = rkyv::to_bytes::<_, 1024>(&self.meta)?;
        let crypt = rkyv::to_bytes::<_, 1024>(&self.crypt)?;

        let meta = self
            .cipher
            .encrypt(&self.crypt.meta_nonce.into(), meta.as_slice())?;

        std::fs::write(&self.config.meta, meta).context(FsError {
            path: self.config.meta.to_string_lossy(),
        })?;
        std::fs::write(&self.config.crypt, &crypt).context(FsError {
            path: self.config.crypt.to_string_lossy(),
        })?;

        Ok(())
    }

    pub fn get_size(&self, record: &Record) -> Result<usize> {
        match &record.inner {
            RecordInner::Empty => Ok(0),
            RecordInner::File(file) => Ok(file.size),
            RecordInner::Directory(dir) => dir
                .entries
                .values()
                .map(|&record_id| self.get_size(&self.meta.entries[record_id]))
                .sum(),
            RecordInner::Symlink(symlink) => {
                let record = &self.meta.entries[symlink.reference_record_id];

                if symlink.is_file {
                    Ok(record.as_file()?.size)
                } else {
                    self.get_size(record)
                }
            }
        }
    }

    fn merge_free(&mut self) {
        let mut free_fragments = self
            .meta
            .free_fragments
            .iter()
            .flat_map(|(size, offsets)| offsets.iter().map(|offset| (*offset, *size)))
            .collect::<Vec<_>>();

        free_fragments.sort_unstable();

        let mut index = 0;
        let mut last_off = 0;
        let mut current_size = 0;

        let mut new_free = Vec::new();

        while let Some(&(off, size)) = free_fragments.get(index) {
            if current_size == 0 {
                last_off = off;
                current_size = size;
            }

            if let Some(&(next_off, next_size)) = free_fragments.get(index + 1) {
                if off + size == next_off {
                    current_size += next_size;
                } else {
                    new_free.push((last_off, current_size));
                    current_size = 0;
                }
            }

            index += 1;
        }

        new_free.push((last_off, current_size));

        let mut fragments = HashMap::<_, Vec<_>>::new();

        for (off, size) in new_free {
            fragments
                .entry(size)
                .and_modify(|offs| offs.push(off))
                .or_insert_with(|| vec![off]);
        }

        self.meta.free_fragments = fragments.into_iter().collect::<Vec<_>>();
        self.meta.free_fragments.sort_unstable();
    }

    pub fn create(&mut self, name: &str, contents: Option<Vec<u8>>) -> Result<Record> {
        let inner = if let Some(contents) = contents {
            let checksum = xxh3_64(&contents);
            let mut nonce = [0; 12];
            OsRng.fill_bytes(&mut nonce);

            let size = contents.len();

            let enc = self.cipher.encrypt(&nonce.into(), contents.as_slice())?;
            let compressed = self.compressor.compress_vec(&enc)?;
            let (len, is_compressed) = if compressed.len() < enc.len() {
                (compressed.len(), true)
            } else {
                (enc.len(), false)
            };

            let free_pos = self
                .meta
                .free_fragments
                .binary_search_by(|&(x, _)| x.cmp(&len))
                .unwrap_or_else(|pos| pos);

            let offset = if let Some((_, ids)) = self.meta.free_fragments.get_mut(free_pos) {
                if let Some(off) = ids.pop() {
                    off
                } else {
                    self.meta.free_fragments.remove(free_pos);
                    let off = self.meta.end_offset;
                    self.meta.end_offset += len;

                    off
                }
            } else {
                let off = self.meta.end_offset;
                self.meta.end_offset += len;

                off
            };

            self.backend.seek(SeekFrom::Start(offset as u64))?;
            self.backend.write_all(&compressed)?;

            RecordInner::File(FileRecord {
                checksum,
                offset,
                len,
                nonce,
                is_compressed,
                size,
            })
        } else {
            RecordInner::Directory(DirectoryRecord {
                entries: HashMap::new(),
            })
        };

        let record = Record {
            id: 0,
            name: name.to_string(),
            file_times: filetime::now(),
            inner,
        };

        let record_id = if let Some(id) = self.meta.empty_records.pop() {
            self.meta.entries[id] = record.clone();

            id
        } else {
            self.meta.entries.push(record.clone());

            self.meta.entries.len() - 1
        };

        self.working_dir_mut()?
            .entries
            .insert(name.to_string(), record_id);

        self.meta.entries[record_id].id = record_id;

        Ok(Record {
            id: record_id,
            ..record
        })
    }

    pub fn read_file(&mut self, record_id: usize) -> Result<Vec<u8>> {
        let record = &mut self.meta.entries[record_id];
        record.file_times.set_accessed();

        let file_record = record.as_file()?;
        let mut buf = vec![0; file_record.len];
        self.backend
            .seek(SeekFrom::Start(file_record.offset as u64))?;
        self.backend.read_exact(&mut buf)?;

        let decompressed = if file_record.is_compressed {
            self.decompressor.decompress_vec(&buf)?
        } else {
            buf
        };

        let contents = self
            .cipher
            .decrypt(&file_record.nonce.into(), decompressed.as_slice())?;

        if xxh3_64(&contents) == file_record.checksum {
            return Ok(contents);
        }

        Err(Error::CorruptedData {
            name: record.name.clone(),
            id: record.id,
        })
    }

    pub fn read_directory(&mut self, record_id: usize) -> Result<Vec<Record>> {
        let record = &mut self.meta.entries[record_id];
        record.file_times.set_accessed();

        let dir_record = record.as_directory()?.clone();

        Ok(dir_record
            .entries
            .values()
            .map(|&id| self.meta.entries[id].clone())
            .collect())
    }

    pub fn delete(&mut self, record_id: usize) -> Result<()> {
        let record = self.meta.entries[record_id].clone();

        match record.inner {
            RecordInner::File(file_record) => {
                self.backend
                    .seek(SeekFrom::Start(file_record.offset as u64))?;
                self.backend.write_all(&vec![0; file_record.len])?;

                if let Some((_, free)) = self
                    .meta
                    .free_fragments
                    .iter_mut()
                    .find(|(size, _)| *size == file_record.len)
                {
                    free.push(file_record.offset);
                } else {
                    let free_pos = self
                        .meta
                        .free_fragments
                        .binary_search_by(|&(x, _)| x.cmp(&file_record.len))
                        .unwrap_or_else(|pos| pos);

                    self.meta
                        .free_fragments
                        .insert(free_pos, (file_record.len, Vec::new()));
                }
            }

            RecordInner::Directory(dir_record) => {
                let orig_id = self.working_dir_id;
                self.set_working_dir_id(record.id);

                for id in dir_record.entries.values() {
                    self.delete(*id)?;
                }

                self.set_working_dir_id(orig_id);
            }

            _ => {}
        }

        self.working_dir_mut()?
            .entries
            .remove(&record.name)
            .context(NotFoundError { name: &record.name })?;

        self.meta.empty_records.push(record_id);
        self.meta.pinned.remove(&record_id);
        self.meta.entries[record_id] = Record::default();

        self.merge_free();

        Ok(())
    }

    pub fn send(&mut self, record_id: usize, to: &[String]) -> Result<()> {
        let mut record = self.meta.entries[record_id].clone();

        self.working_dir_mut()?
            .entries
            .remove(&record.name)
            .context(NotFoundError { name: &record.name })?;

        record.file_times = filetime::now();

        if to.is_empty() {
            self.meta.entries[0]
                .as_directory_mut()?
                .entries
                .insert(record.name.clone(), record.id);

            return Ok(());
        }

        let mut to_dir = 0;

        for part in to {
            to_dir = *self.meta.entries[to_dir]
                .as_directory()?
                .entries
                .get(part)
                .context(NotFoundError { name: &record.name })?;
        }

        self.meta.entries[to_dir]
            .as_directory_mut()?
            .entries
            .insert(record.name.clone(), record.id);

        Ok(())
    }

    pub fn rename(&mut self, old_name: &str, new_name: &str) -> Result<()> {
        let id = self
            .working_dir_mut()?
            .entries
            .remove(old_name)
            .context(NotFoundError { name: old_name })?;

        self.working_dir_mut()?
            .entries
            .insert(new_name.to_owned(), id);

        self.meta.entries[id].name = new_name.to_owned();
        self.meta.entries[id].file_times.set_modified();

        Ok(())
    }

    pub fn get_pinned(&self) -> Vec<Record> {
        self.meta
            .pinned
            .iter()
            .map(|id| self.meta.entries[*id].clone())
            .collect()
    }
}
