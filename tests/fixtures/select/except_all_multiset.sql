-- EXCEPT ALL emits max(0, left_count - right_count) copies of each distinct row.
-- 1: max(0, 3 - 1) = 2 copies; 2: max(0, 2 - 3) = 0 copies.
-- @setup
CREATE TABLE ea_l (a INT);
INSERT INTO ea_l VALUES (1), (1), (1), (2), (2);
CREATE TABLE ea_r (a INT);
INSERT INTO ea_r VALUES (1), (2), (2), (2);
-- @test
SELECT a FROM ea_l
EXCEPT ALL
SELECT a FROM ea_r
ORDER BY a;
