-- A foreign key across two types of one B-tree family is accepted through the family's
-- cross-type equality: `integer_ops` (both directions), `float_ops`, `datetime_ops`.
-- @setup
CREATE TABLE fpi4 (k INT PRIMARY KEY);
CREATE TABLE fpi8 (k BIGINT PRIMARY KEY);
CREATE TABLE fpf4 (k REAL PRIMARY KEY);
CREATE TABLE fpd (k DATE PRIMARY KEY);
CREATE TABLE fpts (k TIMESTAMP PRIMARY KEY);
CREATE TABLE fc8 (k BIGINT REFERENCES fpi4);
CREATE TABLE fc2 (k SMALLINT REFERENCES fpi4);
CREATE TABLE fc4 (k INT REFERENCES fpi8);
CREATE TABLE fcf8 (k FLOAT8 REFERENCES fpf4);
CREATE TABLE fcts (k TIMESTAMPTZ REFERENCES fpd);
CREATE TABLE fcd (k DATE REFERENCES fpts);

-- @test
SELECT conname, contype FROM pg_constraint WHERE contype = 'f' ORDER BY conname;
