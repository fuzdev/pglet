-- @no-txn
-- A `CREATE INDEX` whose *storage build* fails must leave nothing behind — no catalog
-- entry, no storage index def — so the table stays writable. pglet registered the def
-- before back-filling and never unwound, after which a write whose row reached the
-- index was refused against an index the user never got.
--
-- The trigger is the one statement-shaped failure left inside the storage build: a
-- `jsonb` GIN token longer than a B+tree page holds as a key (`EDGES.md` row 192). The
-- executor's entry sizing reads a GIN index's array elements and not a `jsonb` key's
-- tokens, so this passes every gate and the back-fill itself refuses it. The second
-- `INSERT` holds a token as long: a surviving def would refuse that row.
--
-- What this fixture can and cannot see. The catalog half is out of reach twice over:
-- the harness's isolating `BEGIN … ROLLBACK` restores the snapshot, and the
-- `DO … EXCEPTION` swallow the setup needs — the harness propagates the first error
-- from every setup statement, so the refusal has to be caught — sheds it too. So this
-- pins the **storage** half only; the catalog half is `crates/pglet`'s
-- `a_failed_create_index_build_leaves_no_phantom`, which also asserts this statement's
-- `54000` — the fixture cannot say whether the build failed, the compared rows being
-- the same either way — and `the_gin_token_refusal_is_the_storage_builds_own` there is
-- what holds the refusal inside the storage build.
--
-- `@no-txn` is required rather than stylistic: pglet defers index maintenance inside a
-- transaction block, so the same script under the harness's wrapper is green with the
-- defect present.
--
-- 18.6 hashes a long token and builds the index, so its `DO` catches nothing; the
-- compared rows are the same either way, which is the point — the engines agree on the
-- table's contents whatever each did with the index.
-- @setup
DROP TABLE IF EXISTS kbw CASCADE;
CREATE TABLE kbw (a INT, j JSONB NOT NULL);
INSERT INTO kbw VALUES (1, ('"' || lpad('', 5000, 'x') || '"')::jsonb);
DO $$ BEGIN
    CREATE INDEX kbw_ix ON kbw USING gin (j);
EXCEPTION WHEN others THEN NULL;
END $$;
INSERT INTO kbw VALUES (2, ('"' || lpad('', 5000, 'y') || '"')::jsonb);

-- @test
SELECT a, length(j::text) AS n FROM kbw ORDER BY a;
