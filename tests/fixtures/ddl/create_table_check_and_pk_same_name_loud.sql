-- Postgres keeps one unique index on `pg_constraint (conrelid, conname)` spanning
-- CHECK / FOREIGN KEY / PRIMARY KEY / UNIQUE, so the kinds do not get separate name
-- pools: a PRIMARY KEY named after this table's CHECK is 42710. pglet used to scan
-- the two halves independently — checks+FKs on one side, indexes on the other — and
-- every cross-kind pair slipped through, leaving `information_schema` with a
-- duplicated `(table_name, constraint_name)` and a dump stock Postgres refuses.
-- @expect-error: constraint n for relation ctck already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctck (a INT NOT NULL, CONSTRAINT n CHECK (a > 0), CONSTRAINT n PRIMARY KEY (a));
