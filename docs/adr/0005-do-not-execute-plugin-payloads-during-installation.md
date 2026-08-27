# Do not execute plugin payloads during installation

Plugin installation performs manifest, integrity, target, containment, and file-shape validation without starting downloaded executables. Release CI and explicitly isolated end-to-end tests establish that a Hook Plugin is runnable, because executing an untrusted payload merely to validate installation would cross the code-execution boundary before the user can inspect or enable it.

## Consequences

- Static validation cannot claim that a binary implements its declared Hook Protocol.
- RTK release tests must execute the installed absolute path and temporarily project its directory into the test process `PATH` without changing the system environment.
