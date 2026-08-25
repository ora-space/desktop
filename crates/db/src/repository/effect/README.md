# SQLite Effect repository

This module implements the `ora-effect` persistence and source ports. The public adapter owns
transaction boundaries for Desired CAS, source publication and propagation, surface lifecycle,
operation finalization, and retry wakeups. `mapping` contains row validation, normalized inserts,
enum encodings, and request-upsert helpers shared by those transactions.

Desired replacement, source reference protection, and operation finalization use immediate write
transactions so checks cannot race their corresponding writes. Observed and Preserved scans never
enter this module because they remain live filesystem facts.
