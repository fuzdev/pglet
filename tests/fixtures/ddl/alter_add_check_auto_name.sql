-- An unnamed `ALTER TABLE … ADD CHECK` gets a real, stored name. No unnamed CHECK was
-- ever claimed on any route, so two of them landed under one name — and, because the
-- name was synthesized lazily at *display* time, it was synthesized three different
-- ways (the violation message, `information_schema`, the dump) and agreed nowhere.
-- @setup
CREATE TABLE aacan (a INT);
ALTER TABLE aacan ADD CHECK (a > 0);
ALTER TABLE aacan ADD CHECK (a < 9);

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'aacan' AND constraint_type = 'CHECK'
ORDER BY constraint_name;
