-- @setup
-- A `public.`-qualified table name resolves to the base table: `public` is
-- pglet's one implicit schema, so the prefix strips. (Previously any qualifier was
-- silently dropped to the last part, which mis-resolved a non-public schema.)
CREATE TABLE pubt (id INT, label TEXT);
INSERT INTO pubt VALUES (1, 'a'), (2, 'b');

-- @test
SELECT id, label FROM public.pubt WHERE id > 0 ORDER BY id;
