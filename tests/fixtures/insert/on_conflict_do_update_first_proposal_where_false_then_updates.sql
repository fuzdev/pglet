-- A row the statement met but did not write is not refused. The first proposal's `WHERE` is
-- false, so it leaves the row as it was; the second proposal meets the same row, and 18.6
-- updates it. A refusal that fires on every row the statement's probe *met*, rather than
-- on every row it *wrote*, refuses this statement.
-- @profile-lock
-- @setup
CREATE TABLE ocfw (id INT PRIMARY KEY, v TEXT);
INSERT INTO ocfw VALUES (1, 'a');

-- @test
INSERT INTO ocfw VALUES (1, 'x'), (1, 'y')
ON CONFLICT (id) DO UPDATE SET v = excluded.v WHERE excluded.v = 'y'
RETURNING *;
