-- EXCEPT (without ALL) keeps rows in the left side but not the right, as a set: 1 and
-- 3 remain (2 is removed), and the duplicate 1 collapses to one.
-- @setup
CREATE TABLE ed_l (a INT);
INSERT INTO ed_l VALUES (1), (1), (2), (3);
CREATE TABLE ed_r (a INT);
INSERT INTO ed_r VALUES (2);
-- @test
SELECT a FROM ed_l
EXCEPT
SELECT a FROM ed_r
ORDER BY a;
