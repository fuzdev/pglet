-- A composite PRIMARY KEY (source_id, name) — the spine's `cell_field` shape —
-- rejects a full-tuple duplicate. Distinct-on-one-column rows already inserted.
-- @profile-lock
-- @setup
CREATE TABLE cell_field (source_id TEXT, name TEXT, val INT, PRIMARY KEY (source_id, name));
INSERT INTO cell_field VALUES ('s1', 'a', 1), ('s1', 'b', 2), ('s2', 'a', 3);

-- @test
-- @expect-error: duplicate composite primary key (s1, 'a')
INSERT INTO cell_field VALUES ('s1', 'a', 9);
