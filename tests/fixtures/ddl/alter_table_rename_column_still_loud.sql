-- `RENAME COLUMN` is refused `0A000` here; 18.7 renames the column, so any error passes.
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @expect-error: ALTER TABLE … RENAME of a column or a constraint is not supported
-- @test
ALTER TABLE rp RENAME COLUMN v TO w;
