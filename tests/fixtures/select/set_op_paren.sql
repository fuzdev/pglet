-- Parentheses override the default left-associative grouping of EXCEPT. With A={1,2,3},
-- B={2}, C={1}: `A EXCEPT (B EXCEPT C)` = A EXCEPT {2} = {1, 3}, whereas the default
-- `(A EXCEPT B) EXCEPT C` would be {3}. The result {1,3} confirms the parenthesized
-- right operand is grouped first.
-- @setup
CREATE TABLE sp_a (n INT);
INSERT INTO sp_a VALUES (1), (2), (3);
CREATE TABLE sp_b (n INT);
INSERT INTO sp_b VALUES (2);
CREATE TABLE sp_c (n INT);
INSERT INTO sp_c VALUES (1);
-- @test
SELECT n FROM sp_a
EXCEPT
(SELECT n FROM sp_b EXCEPT SELECT n FROM sp_c)
ORDER BY n;
