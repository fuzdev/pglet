-- Index point read with no matching row: `WHERE k = <absent value>` must return
-- the empty set, exactly as Postgres does (the index probe finds nothing).
-- @profile-lock
-- @setup
CREATE TABLE idx_zero (id INT, k INT);
INSERT INTO idx_zero VALUES (1, 10), (2, 20);
CREATE INDEX idx_zero_k ON idx_zero(k);
-- @test
SELECT id, k FROM idx_zero WHERE k = 999 ORDER BY id;
