-- `id = ANY($1::uuid[])` with a bound uuid[] parameter (the spine's load-many).
-- @setup
CREATE TABLE p_cells (id uuid);
INSERT INTO p_cells (id) VALUES
  ('00000000-0000-0000-0000-0000000000a1'),
  ('00000000-0000-0000-0000-0000000000b2'),
  ('00000000-0000-0000-0000-0000000000c3');
-- @param $1 uuid[] {00000000-0000-0000-0000-0000000000a1,00000000-0000-0000-0000-0000000000c3}
-- @test
SELECT id FROM p_cells WHERE id = ANY($1::uuid[]) ORDER BY id;
