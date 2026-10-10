-- INTERSECT ALL emits min(left_count, right_count) copies of each distinct row.
-- 1: min(3, 2) = 2 copies; 2: min(1, 2) = 1 copy.
-- @setup
CREATE TABLE ia_l (a INT);
INSERT INTO ia_l VALUES (1), (1), (1), (2);
CREATE TABLE ia_r (a INT);
INSERT INTO ia_r VALUES (1), (1), (2), (2);
-- @test
SELECT a FROM ia_l
INTERSECT ALL
SELECT a FROM ia_r
ORDER BY a;
