-- Untyped `WHERE id = $1` over a uuid column: the parameter type is inferred
-- from the column (no cast), the case the fuz spine relies on for every lookup.
-- @setup
CREATE TABLE p_users (id uuid, name text);
INSERT INTO p_users (id, name) VALUES
  ('11111111-1111-1111-1111-111111111111', 'alice'),
  ('22222222-2222-2222-2222-222222222222', 'bob');
-- @param $1 uuid 11111111-1111-1111-1111-111111111111
-- @test
SELECT name FROM p_users WHERE id = $1;
