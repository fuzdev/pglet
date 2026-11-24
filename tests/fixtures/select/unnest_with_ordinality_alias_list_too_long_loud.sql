-- The ordinality column is one the item provides, so an alias list one longer than the
-- arrays is accepted and two longer is `42P10`, with the count that includes it.
-- @expect-error: more aliases than the unnest and its ordinality provide
-- @expect-message
-- @test
SELECT * FROM unnest(ARRAY[1, 2]) WITH ORDINALITY AS h(x, o, z);
