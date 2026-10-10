-- The accept side of the identity rule. The first proposal of key 1 moves the row it updated
-- onto key 11, so the second proposal of key 1 conflicts with nothing and inserts: no error on
-- 18.6, two rows. A refusal keyed on a key proposed twice refuses this statement.
-- @profile-lock
-- @setup
CREATE TABLE ocmf (k INT PRIMARY KEY, v TEXT);
INSERT INTO ocmf VALUES (1, 'a');

-- @test
INSERT INTO ocmf VALUES (1, 'x'), (1, 'y')
ON CONFLICT (k) DO UPDATE SET k = ocmf.k + 10, v = excluded.v
RETURNING *;
