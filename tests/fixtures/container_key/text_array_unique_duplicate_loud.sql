-- `tags text[] UNIQUE` refuses the same tags in the same order.
-- @profile-lock
-- @setup
CREATE TABLE cku_t (id INT, tags TEXT[] UNIQUE);
INSERT INTO cku_t (tags, id) VALUES ('{a,b}', 1), ('{b,a}', 2);
-- @test
-- @expect-error: a duplicate text[] key (23505)
-- @expect-message
INSERT INTO cku_t (tags, id) VALUES ('{a,b}', 3);
