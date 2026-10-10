-- The array comparison **writes**. Both orders delete exactly one row, so the command
-- tag is `DELETE 1` either way and only the surviving rows say which: member position
-- destroys `sad`, the label bytes destroy `happy`.
--
-- A value fixture rather than an `@expect-error` one for the reason
-- `check_constraint_member_position` states: the surviving rows are the assertion.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'happy'), (3, 'ok');
DELETE FROM t WHERE ARRAY[m] < ARRAY['ok'::mood];

-- @test
SELECT id, m::text AS label FROM t ORDER BY id;
