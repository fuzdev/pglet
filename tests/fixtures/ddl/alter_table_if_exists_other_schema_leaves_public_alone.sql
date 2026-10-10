-- The skip under another schema's qualifier does not touch a `public` table of the same
-- name — `"Public"` is another schema too, the quoted spelling not folding — while the
-- `public.` spelling names the table and performs the action.
-- @setup
CREATE TABLE atie41 (a INT);
ALTER TABLE IF EXISTS nosuchschema_atie41.atie41 ADD COLUMN b INT;
ALTER TABLE IF EXISTS "Public".atie41 ADD COLUMN c INT;
ALTER TABLE IF EXISTS nosuchschema_atie41.atie41 RENAME TO atie41b;
ALTER TABLE IF EXISTS public.atie41 ADD COLUMN d INT;
-- @test
SELECT column_name::text AS column_name FROM information_schema.columns WHERE table_name = 'atie41'
ORDER BY ordinal_position;
