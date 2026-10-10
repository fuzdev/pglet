-- A `SERIAL`'s `nextval` is a default: the identity clause over the column is refused, the
-- rows it holds drawn from a sequence the clause would not continue.
-- @expect-error: the column already has a default (55000)
-- @expect-message
-- @setup
CREATE TABLE aid (c SERIAL, v TEXT);
INSERT INTO aid (v) VALUES ('a'), ('b');
-- @test
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
