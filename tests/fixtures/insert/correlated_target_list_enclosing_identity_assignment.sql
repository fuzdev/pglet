-- The assignment gate reads a source scalar subquery naming an enclosing level's `int`
-- column as `int4`, so the rows insert — over an inner table whose same-named column is
-- `text`, which read instead would be `42804` (`EDGES.md` row 159).
-- @setup
CREATE TABLE eid_p (a INT, b INT);
CREATE TABLE eid_q3 (c INT, b TEXT);
CREATE TABLE eid_t (z INT);
INSERT INTO eid_p VALUES (1, 10), (2, 20);
INSERT INTO eid_q3 VALUES (5, 'zz');
-- @test
INSERT INTO eid_t SELECT (SELECT eid_p.b FROM eid_q3) FROM eid_p RETURNING z;
