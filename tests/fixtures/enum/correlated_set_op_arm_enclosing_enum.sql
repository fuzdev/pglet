-- A set operation inside a sub-plan whose arm names an enclosing level's enum column
-- unifies that arm as the enum — beside an enum arm, with and without a `FROM`, over an
-- arm lacking the name, under `IN` and `EXISTS`, and through a nested set operation — and
-- orders by member position. The identity rides the record the statement's second parse
-- builds the node with; read as `text`, each would be `42804` (`EDGES.md` row 159).
-- @setup
CREATE TYPE eso_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE eso_ja (x INT, m eso_mood);
CREATE TABLE eso_jb (x INT, m eso_mood);
CREATE TABLE eso_tz (z TEXT);
INSERT INTO eso_ja VALUES (1, 'happy'), (2, 'sad');
INSERT INTO eso_jb VALUES (1, 'ok'), (3, 'sad');
INSERT INTO eso_tz VALUES ('a');
-- @test
SELECT x,
       (SELECT b.m FROM eso_jb b UNION SELECT a.m FROM eso_jb ORDER BY 1 DESC LIMIT 1)::text AS u,
       (SELECT b.m FROM eso_jb b EXCEPT SELECT a.m FROM eso_jb ORDER BY 1 DESC LIMIT 1)::text AS e,
       (SELECT b.m FROM eso_jb b INTERSECT SELECT a.m FROM eso_jb ORDER BY 1 LIMIT 1)::text AS i,
       (SELECT b.m FROM eso_jb b UNION SELECT a.m ORDER BY 1 DESC LIMIT 1)::text AS no_from,
       (SELECT a.m FROM eso_tz UNION SELECT b.m FROM eso_jb b ORDER BY 1 DESC LIMIT 1)::text AS no_local,
       (SELECT b.m FROM eso_jb b UNION SELECT a.m FROM eso_jb INTERSECT SELECT 'happy'
         ORDER BY 1 DESC LIMIT 1)::text AS nested,
       a.m IN (SELECT b.m FROM eso_jb b UNION SELECT a.m FROM eso_jb) AS in_union,
       EXISTS (SELECT b.m FROM eso_jb b EXCEPT SELECT a.m FROM eso_jb) AS exists_except
  FROM eso_ja a ORDER BY x;
