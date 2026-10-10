-- `RETURNING *` over a **derived table** source — the namespace's second node kind.
--
-- A derived table names no catalog relation, so its columns can only come from the
-- plan's own output list. `RETURNING *` therefore reaches a different arm of the
-- relation walk than a base table does, and the same arm the `SET`/`WHERE` scope uses;
-- the qualified spelling `d.zz` rides the same registration.
-- @setup
CREATE TABLE rdw_t (id INT, v TEXT);
INSERT INTO rdw_t VALUES (1, 'T');

-- @test
UPDATE rdw_t SET id = 1 FROM (SELECT 9 AS zz, 'D' AS dv) d WHERE d.zz = 9 RETURNING *;
