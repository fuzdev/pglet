-- The accept side of the self-reference: a `bigint` column referencing the table's own `int` key.
-- @setup
CREATE TABLE fts (id INT PRIMARY KEY, parent BIGINT REFERENCES fts);
ALTER TABLE fts ADD COLUMN up SMALLINT REFERENCES fts;

-- @test
SELECT conname, contype FROM pg_constraint WHERE contype = 'f' ORDER BY conname;
