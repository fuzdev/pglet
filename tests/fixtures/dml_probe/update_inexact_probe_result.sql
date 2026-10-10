-- The rows `update_inexact_probe_declines` leaves behind: `1.5` touched nothing, `1.0` and
-- `1.00` the row 1, `> 1.5` the row 2, and the NULL-keyed row none of them.
-- @profile-lock
-- @setup
CREATE TABLE ip2 (k INT, v TEXT);
CREATE INDEX ip2_k ON ip2 (k);
INSERT INTO ip2 VALUES (1, 'a'), (2, 'b'), (NULL, 'n');
-- @test
UPDATE ip2 SET v = v || 'p' WHERE k = 1.5;
UPDATE ip2 SET v = v || 'q' WHERE k = 1.0;
DELETE FROM ip2 WHERE k = 1.00 AND v = 'aq';
UPDATE ip2 SET v = v || 's' WHERE k > 1.5;
SELECT k, v FROM ip2 ORDER BY k NULLS LAST;
