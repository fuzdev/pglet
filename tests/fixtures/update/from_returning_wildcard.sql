-- `UPDATE t o SET … FROM other a WHERE … RETURNING o.*` — the qualified RETURNING
-- wildcard expands to the target table's columns. Wrapped in a CTE so the body can
-- `ORDER BY` for a deterministic comparison (bare `UPDATE … RETURNING` has no order).
-- @setup
CREATE TABLE offer (id INT, to_account INT, superseded BOOL);
INSERT INTO offer VALUES (1, 9001, false), (2, 9002, false), (3, 9001, false);
CREATE TABLE actor (id INT, account_id INT);
INSERT INTO actor VALUES (100, 9001), (101, 9002);
-- @test
WITH updated AS (
  UPDATE offer o
  SET superseded = true
  FROM actor a
  WHERE a.id = 100 AND o.to_account = a.account_id
  RETURNING o.*
)
SELECT * FROM updated ORDER BY id;
