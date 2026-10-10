-- The other side of the same rule: a `::text` / `::varchar` wrap is *not* an error —
-- Postgres coerces those to `regclass` at run time and answers normally, so the
-- look-through has to survive the coercion check that guards `::int`. (`::regclass`
-- itself is refused in an expression, but by the sequence-name look-through and not the
-- type resolver: `parse_cast_target` lowers the cast to a `StringFn`, so
-- `nextval('s'::regclass)` is pglet's own `0A000` where 18.6 answers — a recorded
-- divergence. The spelling pg_dump emits is a column `DEFAULT`, which a different
-- parser accepts.)
-- @setup
CREATE SEQUENCE nnctt_seq;
-- @test
SELECT nextval('nnctt_seq'::text) AS a, nextval('nnctt_seq'::varchar) AS b;
