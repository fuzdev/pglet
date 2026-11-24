-- A `DELETE` filtered by an `IN` subquery holding a lateral join.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
DELETE FROM lp WHERE id IN (SELECT p.id FROM lp p CROSS JOIN LATERAL (SELECT 1 FROM lc WHERE pid = p.id LIMIT 1) x) RETURNING id;
