-- A composite PRIMARY KEY (source_id, name) allows any two tuples that differ in
-- at least one column: (s1,'a'), (s1,'b') and (s2,'a') all coexist.
-- @setup
CREATE TABLE cell_field (source_id TEXT, name TEXT, val INT, PRIMARY KEY (source_id, name));
INSERT INTO cell_field VALUES ('s1', 'a', 1), ('s1', 'b', 2), ('s2', 'a', 3);

-- @test
SELECT source_id, name, val FROM cell_field ORDER BY source_id, name;
