-- A third component is **surplus**, not out of range, and that is why a window check
-- alone would not have closed it: `numerictypmodin` counts the components and refuses any
-- arity but one or two, where pglet read the first two and ignored the rest — so
-- `1.5::numeric(5,2,3)` answered `1.50`, half-honouring a spelling 18.6 refuses whole.
-- @expect-error: a NUMERIC modifier with three components
-- @expect-message
-- @test
SELECT 1.5::numeric(5,2,3) AS never_reached;
