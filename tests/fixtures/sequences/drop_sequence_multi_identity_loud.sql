-- An identity column's sequence is its INTERNAL implementation, so it is `2BP01` with
-- `CASCADE` as without — and being one name of several does not change that. The
-- refusal is phase 2, which is what
-- `drop_sequence_multi_missing_beats_identity_loud.sql` mirrors with a different
-- SQLSTATE over the same first name.
-- @setup
CREATE SEQUENCE dsi_plain;
CREATE TABLE dsi (id INT GENERATED ALWAYS AS IDENTITY, b INT);

-- @expect-error: cannot drop sequence dsi_id_seq because column id of table dsi requires it
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP SEQUENCE dsi_plain, dsi_id_seq CASCADE;
