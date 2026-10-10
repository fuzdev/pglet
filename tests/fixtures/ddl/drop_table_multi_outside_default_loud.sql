-- The other polarity of `drop_table_multi_in_set_default.sql`: the defaulting column's
-- table is NOT in the list, so the `pg_attrdef` NORMAL edge still refuses (`2BP01`).
-- Widening the deletion set to the whole statement must not widen it past the names
-- written.
-- @setup
CREATE TABLE osa (id SERIAL, v INT);
CREATE TABLE osb (x INT DEFAULT nextval('osa_id_seq'));
CREATE TABLE osc (z INT);

-- @expect-error: cannot drop desired object(s) because other objects depend on them
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE osa, osc;
