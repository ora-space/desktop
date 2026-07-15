import { setupWorker } from "msw/browser";
import { handlers } from "./handlers.js";

/** The browser worker instance shared by all mock transports on the page. */
export const worker = setupWorker(...handlers);
