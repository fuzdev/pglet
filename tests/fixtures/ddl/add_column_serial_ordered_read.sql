-- An `ORDER BY` over an index on an added serial column: every row, in the column's
-- order. The column is declared `NOT NULL`, so the read may be served from the index.
-- @profile-lock
-- @setup
CREATE TABLE acs (v TEXT);
INSERT INTO acs VALUES ('c'), ('a'), ('b');
ALTER TABLE acs ADD COLUMN id SERIAL;
CREATE INDEX acs_id ON acs (id);
INSERT INTO acs (v) VALUES ('d');
-- @test
SELECT id, v FROM acs ORDER BY id DESC;
