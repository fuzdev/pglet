-- `'…'::regclass` names a sequence, as `to_regclass` does: a plain one and a `SERIAL`'s, read through `::text` (EDGES row 85).
-- @setup
CREATE SEQUENCE rgs_plain;
CREATE TABLE rgs_t (id serial);
-- @test
SELECT 'rgs_plain'::regclass::text AS plain, 'RGS_T_ID_SEQ'::regclass::text AS owned;
