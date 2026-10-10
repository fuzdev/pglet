-- ON CONFLICT (pk) DO UPDATE updates the conflicting row (found via the unique
-- index) rather than inserting a duplicate.
-- @profile-lock
-- @setup
CREATE TABLE pk_upsert (id INT PRIMARY KEY, name TEXT);
INSERT INTO pk_upsert VALUES (1, 'Alice'), (2, 'Bob');
INSERT INTO pk_upsert VALUES (1, 'NewAlice'), (3, 'Carol')
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name;

-- @test
SELECT * FROM pk_upsert ORDER BY id;
