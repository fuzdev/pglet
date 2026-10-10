-- A `USING` merge one join deeper. The merged column belongs to the inner join itself,
-- so no qualifier names it and a bare reference must find exactly it — but the outer
-- level used to re-file it under a placeholder relation beside both sides' own `c`, and
-- a bare `c` counted three claimants and was `42702` where 18.6 answers. Ordinary SQL,
-- no quoted name; loud.
-- @setup
CREATE TABLE num_a (c TEXT, x INT);
CREATE TABLE num_b (c TEXT, y INT);
CREATE TABLE num_n (id INT, v TEXT);
INSERT INTO num_a VALUES ('hit', 7);
INSERT INTO num_b VALUES ('hit', 8);
INSERT INTO num_n VALUES (1, 'n1');

-- @test
SELECT x FROM num_a JOIN num_b USING (c) JOIN num_n n ON n.id = 1 WHERE c = 'hit';
