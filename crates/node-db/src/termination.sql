CREATE TABLE process_terminations (
    run TEXT PRIMARY KEY REFERENCES process_attempts(run),
    signal INTEGER NOT NULL
);
CREATE TRIGGER outcome_excludes_termination BEFORE INSERT ON process_outcomes
WHEN EXISTS(SELECT 1 FROM process_terminations WHERE run=NEW.run)
BEGIN SELECT RAISE(ABORT, 'run already terminated by a signal'); END;
CREATE TRIGGER termination_excludes_outcome BEFORE INSERT ON process_terminations
WHEN EXISTS(SELECT 1 FROM process_outcomes WHERE run=NEW.run)
BEGIN SELECT RAISE(ABORT, 'run already exited with a code'); END;
