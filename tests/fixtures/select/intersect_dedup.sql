-- INTERSECT (without ALL) keeps rows present in both sides as a set: 2 and 3 are in
-- both; the duplicate 1 (left-only) and duplicate 3 (right) collapse to one 3.
-- @setup
CREATE TABLE isect_l (a INT);
INSERT INTO isect_l VALUES (1), (1), (2), (3);
CREATE TABLE isect_r (a INT);
INSERT INTO isect_r VALUES (2), (3), (3), (4);
-- @test
SELECT a FROM isect_l
INTERSECT
SELECT a FROM isect_r
ORDER BY a;
