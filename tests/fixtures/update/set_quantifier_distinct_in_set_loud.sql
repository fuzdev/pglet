-- An `UPDATE … SET` right-hand side. 18.6 refuses in parse analysis and the row keeps
-- its old value; pglet dropped the modifier and stored `A`. 42809 on both.
-- @setup
CREATE TABLE sq_du (id INT, c TEXT);
INSERT INTO sq_du VALUES (1, 'x');
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
UPDATE sq_du SET c = upper(DISTINCT 'a') WHERE id = 1;
