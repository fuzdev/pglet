-- …and it beats an `IF NOT EXISTS` skip, which is a Postgres **success**: a gate standing
-- in for no error at all still loses to a verdict the grammar raised. Before the length
-- window became an `Iconst` this was a *silent skip* here against 18.6's `42601`; the
-- carrier's `22023` does lose to this same skip
-- (`refused_typmod_add_column_if_not_exists_skips`), which is the contrast pair.
-- @setup
CREATE TABLE rtgskip (a INT);
-- @no-error-fields: both engines send a `P` and pglet's reader reports one token later —
--   see `types/refused_typmod_char_length_not_an_integer_loud.sql` for the offset.
-- @expect-error: a grammar verdict outranks even the skip
-- @test
ALTER TABLE rtgskip ADD COLUMN IF NOT EXISTS a CHAR(2147483648);
