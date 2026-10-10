-- The other half of the declared-name rule, and the one a name-convention lookup gets
-- backwards: once the constraint is named, `<table>_pkey` was never minted, so
-- DROP INDEX on it is 42704 (undefined_object) — not the 2BP01 the auto-named case
-- gives. pglet reported those two exactly inverted before the name was kept.
-- @setup
CREATE TABLE dian (a INT NOT NULL, b INT);
ALTER TABLE dian ADD CONSTRAINT dian_my_pk PRIMARY KEY (a);

-- @expect-error: index dian_pkey does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP INDEX dian_pkey;
