-- Where the refusal sits among the second proposal's own checks: 18.6 checks the proposed
-- row's `NOT NULL` and `CHECK` constraints before it probes the arbiter (`ExecInsert` runs
-- `ExecConstraints` ahead of `ExecCheckIndexConstraints`), so the second proposal's NULL is
-- `23502`, ahead of the `21000` its key would draw.
-- @profile-lock
-- @setup
CREATE TABLE ocnn (id INT PRIMARY KEY, v TEXT NOT NULL);

-- @test
-- @expect-error: null value in column "v" violates not-null constraint (23502), ahead of 21000
INSERT INTO ocnn VALUES (1, 'a'), (1, NULL)
ON CONFLICT (id) DO UPDATE SET v = 'z';
