-- A btree index over a `timestamptz` holding both infinities, read by a range bounded on
-- both sides and by `> now()`, the "not yet expired" shape.
-- @profile-lock
-- @setup
CREATE TABLE infix (id INT, ts TIMESTAMPTZ);
CREATE INDEX infix_ts ON infix (ts);
INSERT INTO infix VALUES (1, 'infinity'), (2, '-infinity'), (3, '2000-01-01 00:00:00+00'), (4, NULL), (5, '9999-12-31 00:00:00+00');
-- @test
SELECT id FROM infix WHERE ts > now() OR ts BETWEEN timestamptz '-infinity' AND timestamptz '2000-01-01 00:00:00+00' ORDER BY ts, id;
