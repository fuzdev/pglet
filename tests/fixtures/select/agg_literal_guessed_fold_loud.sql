-- A fold over a column whose type is the walkers' guess is not read at a certain type: the pass's own cast over the
-- guessed column is not taken as the aggregate's type, so the literal stays text where 18.6 reads it.
-- @test
-- @expect-error: the literal stays text beside an aggregate over a fold of a guessed column
SELECT max(COALESCE(x, 1.5e0)) > '1' FROM unnest(ARRAY[1, 2]) u(x);
