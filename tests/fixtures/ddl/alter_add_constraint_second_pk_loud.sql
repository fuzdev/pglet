-- `ADD CONSTRAINT <n> PRIMARY KEY` on a table that already has one is 42P16. The
-- declared name is deliberately one that sorts *before* the existing `aacs_pkey`:
-- with no multiplicity check the second key lands as a second constraint-marked
-- index and `Schema::primary_key_index` — a `find` over a name-sorted map — starts
-- answering with whichever key sorts first, so `REFERENCES aacs` and `GROUP BY`
-- functional dependency both silently move to column `b`.
-- @setup
CREATE TABLE aacs (a INT NOT NULL, b INT NOT NULL, PRIMARY KEY (a));

-- @expect-error: multiple primary keys for table aacs are not allowed
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE aacs ADD CONSTRAINT aaa_first_alphabetically PRIMARY KEY (b);
