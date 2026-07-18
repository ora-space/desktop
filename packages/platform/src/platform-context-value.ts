import { createContext } from "react";
import type { PlatformAdapter } from "./types";

export const PlatformContext = createContext<PlatformAdapter | null>(null);
