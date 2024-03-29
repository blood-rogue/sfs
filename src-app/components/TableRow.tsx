import { useAppStateContext } from "@hooks/useAppStateContext";
import { FileTime } from "@type/FileTime";
import { Record } from "@type/Record";
import { LucideIcon } from "lucide-react";
import { DateTime } from "luxon";

export const TableRow: React.FC<{
    record: Record;
    date: FileTime;
    size: string;
    icon: LucideIcon;
    editing: boolean;
    onInput: (value: string) => void;
}> = ({ record, date, size, icon: Icon, editing, onInput }) => {
    const { appState } = useAppStateContext();

    return (
        <>
            <span className="flex flex-row gap-4">
                <input
                    checked
                    disabled
                    type="checkbox"
                    className={`checkbox checkbox-sm pl-3 transition-opacity duration-200 ${
                        appState.selected.length > 1 &&
                        appState.selected.some((obj) => obj.id === record.id)
                            ? "opacity-50"
                            : "!opacity-0"
                    }`}
                />
                <span className="size-5">
                    <Icon size={18} strokeWidth={1} />
                </span>
                <span
                    className="text-ellipsis mr-2"
                    contentEditable={editing}
                    suppressContentEditableWarning
                    onInput={(ev) =>
                        onInput(ev.currentTarget.textContent || "")
                    }
                >
                    {record.name}
                </span>
            </span>
            <span>{size}</span>
            <span>
                {DateTime.fromObject(date, { zone: "utc" })
                    .setZone("local")
                    .toRelative()}
            </span>
        </>
    );
};
