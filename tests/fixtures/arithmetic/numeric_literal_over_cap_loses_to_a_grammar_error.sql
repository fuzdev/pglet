-- 18.6 raises `numeric_in`'s `22003` in parse analysis, which its raw parse outranks, so
-- a grammar error anywhere in the statement answers first. pglet owes the refusal through
-- the deferred analysis slot for exactly that reason; raising it where the literal is read
-- would answer `22003` here against 18.6's `42601`.
-- @test
-- @expect-error: the grammar error outranks the literal's deferred 22003
SELECT 1e131073 1;
