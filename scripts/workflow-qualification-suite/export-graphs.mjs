import { generateAllWorkflows } from "./generator.mjs";

// The Rust decoder test consumes actual generator output without persistence or Agent execution.
console.log(JSON.stringify(generateAllWorkflows()));
