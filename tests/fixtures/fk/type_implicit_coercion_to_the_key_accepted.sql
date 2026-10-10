-- A foreign key whose referencing type has no equality in the key's family is accepted when
-- both column types coerce implicitly to the key class's input type: `int` and `bigint` to
-- `numeric`, `int` and `numeric` to `float8`, `varchar` to `text` (a `varchar` key's class
-- takes `text` too, so `text` referencing it is accepted), `time` to `interval`.
-- @setup
CREATE TABLE fpn (k NUMERIC PRIMARY KEY);
CREATE TABLE fpf8 (k FLOAT8 PRIMARY KEY);
CREATE TABLE fpt (k TEXT PRIMARY KEY);
CREATE TABLE fpv (k VARCHAR(10) PRIMARY KEY);
CREATE TABLE fpiv (k INTERVAL PRIMARY KEY);
CREATE TABLE fcin (k INT REFERENCES fpn);
CREATE TABLE fcbn (k BIGINT REFERENCES fpn);
CREATE TABLE fcif (k INT REFERENCES fpf8);
CREATE TABLE fcnf (k NUMERIC(6, 2) REFERENCES fpf8);
CREATE TABLE fcvt (k VARCHAR(3) REFERENCES fpt);
CREATE TABLE fctv (k TEXT REFERENCES fpv);
CREATE TABLE fcti (k TIME REFERENCES fpiv);

-- @test
SELECT conname, contype FROM pg_constraint WHERE contype = 'f' ORDER BY conname;
