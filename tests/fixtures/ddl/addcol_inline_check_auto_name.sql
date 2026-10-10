-- An `ADD COLUMN`'s inline CHECK is named after its own column, and each added column
-- gets its own base — `aicad_b_check`, then `aicad_c_check`, no digits, since the bases
-- differ. The unnamed-CHECK naming that used to be synthesized at display time counted
-- only earlier *unnamed* checks sharing a base, so it was invisible to every gate.
-- @setup
CREATE TABLE aicad (a INT);
ALTER TABLE aicad ADD COLUMN b INT CHECK (b > 0);
ALTER TABLE aicad ADD COLUMN c INT CHECK (c > 0);

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'aicad' AND constraint_type = 'CHECK'
ORDER BY constraint_name;
