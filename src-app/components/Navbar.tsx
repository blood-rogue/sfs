import { useAppStateContext } from "@hooks/useAppStateContext";
import { useModalContext } from "@hooks/useModalContext";
import { ActionType } from "@type/ActionType";
import { ModalEnum } from "@type/ModalEnum";
import {
    Download,
    FilePlus,
    FolderPlus,
    FolderUp,
    Info,
    Pin,
    PinOff,
    SendHorizonal,
    Trash2,
} from "lucide-react";

import {
    createColor,
    deleteColor,
    dirActionColor,
    infoColor,
} from "../utils/colors";
import { IconButton } from "./IconButton";

export const Navbar: React.FC = () => {
    const { openModal } = useModalContext();
    const { appState, dispatch } = useAppStateContext();

    const isPinned = appState.pinned.some(
        (record) => record.id === appState.workingDirRecord.id,
    );

    const moveDirUp = () => {
        const path = [...appState.workingDir];
        path.pop();

        const toDir = path.at(-1) || { id: 0, path: [] };

        dispatch({
            type: ActionType.CHANGE_DIRECTORY,
            payload: { id: toDir.id, path },
        });
    };

    return (
        <div className="navbar bg-dark-50 border border-dark-300 shadow-none rounded-lg">
            <div className="navbar-start">
                <input
                    type="text"
                    className="input input-block border bg-dark-50 input-sm"
                    placeholder="Search"
                />
            </div>
            <div className="navbar-end gap-3">
                <span
                    className={`${
                        appState.selected.length > 1
                            ? "opacity-100"
                            : "opacity-0"
                    } flex flex-row gap-3 transition-all ease-in-out duration-200`}
                >
                    <IconButton
                        icon={Trash2}
                        color={deleteColor}
                        tooltipBot="Delete"
                        onClick={() => openModal(ModalEnum.DELETE)}
                    />
                    <IconButton
                        icon={SendHorizonal}
                        color={dirActionColor}
                        tooltipBot="Move To"
                        onClick={() => openModal(ModalEnum.SEND_TO)}
                    />
                </span>
                {appState.workingDirRecord.id > 0 && (
                    <>
                        <IconButton
                            icon={isPinned ? PinOff : Pin}
                            color={dirActionColor}
                            tooltipBot={
                                isPinned ? "Unpin Directory" : "Pin Directory"
                            }
                            onClick={() => {
                                dispatch(
                                    isPinned
                                        ? {
                                              type: ActionType.UNPIN,
                                              payload:
                                                  appState.workingDirRecord.id,
                                          }
                                        : {
                                              type: ActionType.PIN,
                                              payload:
                                                  appState.workingDirRecord,
                                          },
                                );
                            }}
                        />
                        <IconButton
                            icon={FolderUp}
                            color={dirActionColor}
                            tooltipBot="Move Up"
                            onClick={() => moveDirUp()}
                        />
                    </>
                )}
                <IconButton
                    icon={Info}
                    color={infoColor}
                    tooltipBot="Info"
                    onClick={() => openModal(ModalEnum.INFO)}
                />
                <IconButton
                    icon={FolderPlus}
                    color={createColor}
                    tooltipBot="New Directory"
                    onClick={() => openModal(ModalEnum.NEW_DIRECTORY)}
                />
                <IconButton
                    icon={FilePlus}
                    color={createColor}
                    tooltipBot="New File"
                    onClick={() => openModal(ModalEnum.NEW_FILE)}
                />
                <IconButton
                    icon={Download}
                    color={createColor}
                    tooltipBot="Import"
                    onClick={() => dispatch({ type: ActionType.IMPORT })}
                />
            </div>
        </div>
    );
};
