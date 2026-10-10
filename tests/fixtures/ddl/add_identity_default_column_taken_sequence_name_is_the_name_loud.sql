-- A declared `SEQUENCE NAME` a relation holds, over a column that has a default: the
-- name's refusal wins.
-- @expect-error: relation already exists (42P07)
-- @expect-message
-- @setup
CREATE TABLE aid (c INT NOT NULL DEFAULT 5, v TEXT);
CREATE TABLE aid_taken (x INT);
-- @test
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME aid_taken);
