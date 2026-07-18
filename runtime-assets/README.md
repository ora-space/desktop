# Plugin runtime resources

`bun-source-v1.json` freezes the approved upstream Windows Bun archive. The normal build and test
commands never access the network. Run `task prepare-plugin-runtime` explicitly to download and
verify that archive, extract only `bun.exe`, bundle the private bootstrap with that exact Bun, and
create the ignored `runtime-assets/prepared/` application-resource directory plus its strict
`runtime-manifest.json`.

Run `task prepare-plugin-runtime` once before `task test`; the test gate checks this prerequisite
before compiling the Tauri application and never downloads assets implicitly. Both `task test` and
the narrower `task test-plugin-runtime-e2e` exercise the verified Bun through the Windows Job stdio
harness, a public-SDK packed Agent, the complete library management/runtime lifecycle, and
authenticated `BackendRuntime` loopback startup/shutdown. No E2E consumer falls back to a system
`PATH` Bun. The full test gate also regenerates contract artifacts and fails if their tracked
contents drift, leaving explicit export tasks as the only intentional regeneration workflow.
