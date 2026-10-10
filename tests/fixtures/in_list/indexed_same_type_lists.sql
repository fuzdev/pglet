-- Lists already of the left operand's type, over a primary key and an indexed `text` column.
-- @profile-lock
-- @setup
CREATE TABLE inl_t (id INT PRIMARY KEY, t TEXT);
CREATE INDEX inl_t_t ON inl_t (t);
INSERT INTO inl_t VALUES (1, 'a'), (2, 'b'), (3, 'c'), (5, 'd');
-- @test
SELECT id FROM inl_t WHERE id IN (1, 2, 5) AND t IN ('a', 'b', 'c') AND id NOT IN (2, 9) ORDER BY id;
