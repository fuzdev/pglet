-- A GIN-indexed jsonb column under `@>` in an UPDATE's WHERE: the rows that contain the
-- object, in row order.
-- @profile-lock
-- @setup
CREATE TABLE gj (id INT, j JSONB);
CREATE INDEX gj_j ON gj USING gin (j);
INSERT INTO gj VALUES (3, '{"a": 1, "b": 2}'), (1, '{"a": 1}'), (2, '{"a": 2}');
-- @test
UPDATE gj SET id = id + 10 WHERE j @> '{"a": 1}' RETURNING id, j;
