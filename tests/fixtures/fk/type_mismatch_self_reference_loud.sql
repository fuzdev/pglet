-- A self-referential foreign key on `CREATE TABLE` is typed against the table's in-flight
-- columns.
-- @expect-error: 42804 text referencing integer
-- @expect-message
-- @test
CREATE TABLE fts (id INT PRIMARY KEY, parent TEXT REFERENCES fts);
