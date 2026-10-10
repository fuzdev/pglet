-- `DO UPDATE`'s namespace is the target plus `excluded`, so a qualifier naming neither is
-- `42P01` on 18.6 with the conflicting row untouched. pglet **wrote the excluded value**:
-- the reference fell past the outer chain's identity tier into a bare loop that never
-- read the qualifier, and the `excluded` frame — pushed innermost — carried a `v`.
--
-- That is what makes this member worse than its `WHERE`-clause twin: the wrong binding is
-- not merely a refusal not raised, it is the upsert silently taking the branch a correct
-- engine refuses to compile. The accept polarity is `SET v = excluded.v`, which answers
-- identically on both engines.
-- @setup
CREATE TABLE ocb_t (id INT PRIMARY KEY, v TEXT);
INSERT INTO ocb_t VALUES (1, 'orig');

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @no-profile-lock: the arbiter's unique-index probe is constraint enforcement rather than acceleration and runs identically on both profiles; the refusal is raised evaluating the DO UPDATE action, which selects no index
-- @test
INSERT INTO ocb_t VALUES (1, 'new') ON CONFLICT (id) DO UPDATE SET v = zzz.v;
