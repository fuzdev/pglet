-- The refusal of `drop_schema_holding_only_a_sequence_cascade_required_loud` leaves the
-- sequence as it was: drawn to `1`, then a `DROP SCHEMA public` without `CASCADE` that a
-- caught `DO` body raises `2BP01` on, and the next draw answers `2` on 18.6. A drop that
-- went ahead would leave `dsk_s` unresolvable (`42P01`). The setup empties `public`
-- first, inside the isolating wrapper, so no other object stands in for the sequence.
-- @setup
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
CREATE SEQUENCE dsk_s;
SELECT nextval('dsk_s');
DO $$ BEGIN DROP SCHEMA public; EXCEPTION WHEN OTHERS THEN NULL; END $$;
-- @test
SELECT nextval('dsk_s') AS s;
