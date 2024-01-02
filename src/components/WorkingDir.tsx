import { Home } from "lucide-react";

import { useAppStateContext } from "../context";

export const WorkingDir: React.FC = () => {
    const {
        appState: { workingDir },
    } = useAppStateContext();

    return (
        <div className="breadcrumbs text-sm min-h-5">
            <ul>
                <li>
                    <a>
                        <Home strokeWidth={1} size={15} />
                    </a>
                </li>
                {workingDir.map((segment, idx) => (
                    <li key={idx}>
                        <a>{segment}</a>
                    </li>
                ))}
            </ul>
        </div>
    );
};
