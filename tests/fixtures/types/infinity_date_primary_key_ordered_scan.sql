-- A bare `ORDER BY` over a primary key holding both infinities, which the accelerated build
-- answers in index order with no sort above it: the key order must be the value order.
-- @profile-lock
-- @setup
CREATE TABLE infpko (d DATE PRIMARY KEY, v INT);
INSERT INTO infpko VALUES ('infinity', 1), ('2000-01-01', 2), ('-infinity', 3), ('9999-12-31', 4), ('0001-01-01', 5);
-- @test
SELECT d, v FROM infpko ORDER BY d LIMIT 4;
