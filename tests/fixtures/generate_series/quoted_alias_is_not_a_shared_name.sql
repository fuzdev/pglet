-- A quoted alias is compared byte for byte: "G" and g are two names, so a bare `*` over the pair expands both items
-- rather than refusing the shared column name `x` as ambiguous. `first` carries the "G" item's value, the two `x`
-- columns sharing one key in the recorded rows.
-- @test
SELECT *, "G".x AS first FROM generate_series(1, 2) "G"(x), generate_series(5, 6) g(x) ORDER BY first, 2;
