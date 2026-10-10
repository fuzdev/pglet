-- The typed-constant window (`timestamp(3) '…'`) reads the precision too — it lowers
-- to the same cast node the `::` spelling does, so nothing downstream has a second
-- shape to learn. `interval(3) '…'` is **not** here: `interval` is not an
-- `is_multiword_type_lead` word, so that window falls to the function-call path for
-- pglet's own `0A000 function not supported: interval` (recorded, out of this
-- fixture's scope).
-- @test
SELECT timestamp(3) '2020-01-02 03:04:05.6789' AS a,
       timestamptz(3) '2020-01-02 03:04:05.6789' AS b,
       time(3) '03:04:05.6789' AS c;
