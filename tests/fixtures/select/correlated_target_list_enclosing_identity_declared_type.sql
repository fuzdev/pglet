-- A sub-plan's target list naming an **enclosing** level's column is described with that
-- column's type — over a one-column inner table lacking the name, an empty one, a wider
-- one carrying a same-named column of another type, a FROM `unnest`, and computed — not
-- the local same-named column's type, nor `text` (`EDGES.md` row 159); both declared-type
-- legs compare it.
-- @setup
CREATE TABLE eid_p (a INT, b INT);
CREATE TABLE eid_q (c INT);
CREATE TABLE eid_q2 (c INT, d INT);
CREATE TABLE eid_q3 (c INT, b TEXT);
CREATE TABLE eid_e (c INT);
CREATE TABLE eid_v (a INT, v VARCHAR(5), n NUMERIC(6,2), f FLOAT8);
INSERT INTO eid_p VALUES (1, 10), (2, 20);
INSERT INTO eid_q VALUES (5);
INSERT INTO eid_q2 VALUES (5, 6);
INSERT INTO eid_q3 VALUES (5, 'zz');
INSERT INTO eid_v VALUES (1, 'ab', 1.5, 2.5), (2, 'cd', 2.25, 3.5);
-- @test
SELECT eid_p.a,
       (SELECT eid_p.b FROM eid_q) AS qualified,
       (SELECT b FROM eid_q) AS bare,
       (SELECT eid_p.b FROM eid_e) AS empty_inner,
       (SELECT eid_p.b FROM eid_q3) AS local_text_same_name,
       (SELECT b FROM eid_q2) AS wider_inner,
       (SELECT eid_p.b + 1 FROM eid_q3) AS computed,
       (SELECT eid_p.b FROM unnest(ARRAY[1]) u(z)) AS over_unnest,
       (SELECT v FROM eid_q WHERE eid_q.c = 5) AS from_v,
       (SELECT n FROM eid_q) AS from_n,
       (SELECT f FROM eid_q) AS from_f
  FROM eid_p JOIN eid_v USING (a) ORDER BY eid_p.a;
