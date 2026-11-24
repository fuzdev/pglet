-- `JOIN LATERAL … ON <predicate>`: the `ON` filters each left row's pairs.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT p.id, c.v FROM lp p JOIN LATERAL (SELECT v FROM lc WHERE lc.pid = p.id) c ON c.v <> 'x' ORDER BY 1, 2;
