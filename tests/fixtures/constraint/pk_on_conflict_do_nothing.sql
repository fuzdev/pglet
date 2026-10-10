-- ON CONFLICT (pk) DO NOTHING skips the conflicting row and inserts the rest.
-- @profile-lock
-- @setup
CREATE TABLE pk_skip (id INT PRIMARY KEY, name TEXT);
INSERT INTO pk_skip VALUES (1, 'Alice'), (2, 'Bob');
INSERT INTO pk_skip VALUES (1, 'NewAlice'), (3, 'Carol')
ON CONFLICT (id) DO NOTHING;

-- @test
SELECT * FROM pk_skip ORDER BY id;
