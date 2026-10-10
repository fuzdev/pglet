-- A `DELETE` whose `IN` subquery is a range with no lower bound removes the rows the
-- range names; the row named after the NULL-keyed one stays.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
CREATE TABLE sink (v TEXT);
INSERT INTO sink VALUES ('null'), ('one'), ('ten');
-- @test
DELETE FROM sink WHERE v IN (SELECT v FROM rk WHERE c < 5);
SELECT v FROM sink ORDER BY v;
