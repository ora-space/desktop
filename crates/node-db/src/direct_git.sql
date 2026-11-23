CREATE TABLE process_stops (
    run TEXT PRIMARY KEY REFERENCES process_attempts(run)
);
CREATE TRIGGER stop_excludes_other_endings BEFORE INSERT ON process_stops
WHEN EXISTS(SELECT 1 FROM process_outcomes WHERE run=NEW.run)
  OR EXISTS(SELECT 1 FROM process_terminations WHERE run=NEW.run)
BEGIN SELECT RAISE(ABORT, 'run already ended with an exit status'); END;
CREATE TRIGGER outcome_excludes_stop BEFORE INSERT ON process_outcomes
WHEN EXISTS(SELECT 1 FROM process_stops WHERE run=NEW.run)
BEGIN SELECT RAISE(ABORT, 'run already stopped without an exit status'); END;
CREATE TRIGGER termination_excludes_stop BEFORE INSERT ON process_terminations
WHEN EXISTS(SELECT 1 FROM process_stops WHERE run=NEW.run)
BEGIN SELECT RAISE(ABORT, 'run already stopped without an exit status'); END;
INSERT INTO process_stops
SELECT run FROM process_attempts
WHERE cleaned=0
  AND NOT EXISTS(SELECT 1 FROM process_outcomes WHERE process_outcomes.run=process_attempts.run)
  AND NOT EXISTS(SELECT 1 FROM process_terminations WHERE process_terminations.run=process_attempts.run);
UPDATE process_attempts SET cleaned=1 WHERE cleaned=0;
UPDATE restore_process_attempts SET cleaned=1 WHERE cleaned=0;
