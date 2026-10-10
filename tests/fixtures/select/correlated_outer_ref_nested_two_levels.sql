-- A correlated reference reaching **two levels out**: the innermost subquery names a
-- column of the outermost query, past a subquery in between. The descent chains
-- level by level, so the innermost sub-plan is resolved against a three-level scope
-- (its own FROM, the middle query's, the outermost query's) — and the second leg
-- checks the *middle* level is reachable too, which a chain that only ever carried the
-- outermost scope would fail.
-- @setup
CREATE TABLE cn_o (okey INT, ouid UUID);
CREATE TABLE cn_i (ikey INT, iamt REAL);
INSERT INTO cn_o VALUES (1, '11111111-1111-1111-1111-111111111111'), (2, '22222222-2222-2222-2222-222222222222');
INSERT INTO cn_i VALUES (1, 1.1);
-- @test
SELECT okey FROM cn_o
 WHERE EXISTS (SELECT 1 FROM cn_i x WHERE EXISTS (SELECT 1 FROM cn_i y WHERE y.ikey = x.ikey AND cn_o.ouid = '11111111-1111-1111-1111-111111111111'))
   AND EXISTS (SELECT 1 FROM cn_i p WHERE EXISTS (SELECT 1 FROM cn_i q WHERE p.iamt = '1.1'))
 ORDER BY okey;
