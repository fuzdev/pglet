-- The polarity fixture: both windows are **inclusive**. An off-by-one at either end
-- refuses a spelling both engines accept, and an over-reaching gate is the failure this
-- whole family invites — a fixture asserts one statement, so the ends need their own.
--
-- Every cell here has to *answer a value*, which is what keeps the fixture from asserting
-- nothing — and the scale-high end does, at a value the scale can hold: `numeric(5,1000)`
-- is a legal modifier and `0::numeric(5,1000)` is a thousand fractional zeros on both
-- engines, so the end is pinned here beside `numeric(5,-1000)` rather than only at a column
-- definition. A *nonzero* value is what the scale cannot hold (`1.5::numeric(5,1000)` is
-- `22003` on both engines, under pglet's `numeric out of range` against 18.6's
-- `numeric field overflow` — the standing message gap `docs/errors.md` § Known gaps
-- records); including one recorded an error shape and made every other cell in the row
-- vacuous.
-- @test
SELECT 'ab'::varchar(10485760) AS vmax,
       1.5::numeric(1) AS pmin,
       1.5::numeric(1000) AS pmax,
       1.5::numeric(5,-1000) AS smin,
       0::numeric(5,1000) AS smax;
