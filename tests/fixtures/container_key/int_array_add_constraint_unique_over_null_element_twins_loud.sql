-- `ALTER TABLE … ADD CONSTRAINT … UNIQUE` over rows holding one array key twice — two
-- arrays with a NULL element in the same position.
-- @profile-lock
-- @setup
CREATE TABLE ckt_a (id INT, a INT[]);
INSERT INTO ckt_a (a, id) VALUES ('{1,NULL}', 1), ('{1,2}', 2), ('{1,NULL}', 3), (NULL, 4), (NULL, 5);
-- @test
-- @expect-error: the constraint's build finds the repeated array key (23505)
-- @expect-message
ALTER TABLE ckt_a ADD CONSTRAINT ckt_a_u UNIQUE (a);
