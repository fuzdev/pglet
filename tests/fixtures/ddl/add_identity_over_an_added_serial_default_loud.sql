-- A serial column added by `ADD COLUMN` is declared `NOT NULL` and has a default, so the
-- identity clause over it is the default's refusal, not the declaration's.
-- @expect-error: the column already has a default (55000)
-- @expect-message
-- @setup
CREATE TABLE aid (v TEXT);
INSERT INTO aid VALUES ('a');
ALTER TABLE aid ADD COLUMN c SERIAL;
-- @test
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
