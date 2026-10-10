-- A **correlated** enum reference inside a `RETURNING` item's subquery: the just-written
-- row is pushed as the outer frame, so this reads the frame's own member orders rather
-- than the clause's resolver — the second of the two objects `project_returning` builds,
-- and the one `returning_member_position` does not reach.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (k INT);
INSERT INTO u VALUES (1);

-- @test
INSERT INTO t VALUES (1, 'happy'), (2, 'sad')
RETURNING id, (SELECT count(*) FROM u WHERE t.m > 'sad') AS above_sad;
