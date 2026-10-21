// The page in the island window, see Island.tsx.
import { createRoot } from "react-dom/client";
import { Island } from "./Island";

const root = document.getElementById("root");
if (root) createRoot(root).render(<Island />);
