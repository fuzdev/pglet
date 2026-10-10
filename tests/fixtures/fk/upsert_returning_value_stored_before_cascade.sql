-- A sibling data-modifying CTE stores the upsert's `RETURNING` value, which reads the child
-- before the `ON UPDATE CASCADE` repoints it — so the stored value is the old key.
-- @profile-lock
-- @setup
CREATE TABLE urvs_p (id INT PRIMARY KEY, v INT);
CREATE TABLE urvs_c (pid INT REFERENCES urvs_p(id) ON UPDATE CASCADE, x INT);
CREATE TABLE urvs_log (r INT);
INSERT INTO urvs_p VALUES (1, 1), (2, 2);
INSERT INTO urvs_c VALUES (1, 10), (2, 20);
WITH a AS (
  INSERT INTO urvs_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9
  RETURNING (SELECT pid FROM urvs_c WHERE x = 10) AS r
), b AS (INSERT INTO urvs_log SELECT r FROM a)
SELECT 1;

-- @test
SELECT r FROM urvs_log;
