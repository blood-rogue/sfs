import { Modal, ModalProps } from "./Modal";

export const RenameModal: React.FC<ModalProps> = ({ onClick, checked }) => (
    <Modal checked={checked} onClick={onClick} title="Rename" />
);