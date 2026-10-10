-- Bare `<`/`>` over arrays with a NULL element: the NULL element compares GREATER
-- than any non-NULL, so `ARRAY[1,NULL] > ARRAY[1,2]` and `> ARRAY[1,0]`.
-- @test
SELECT
  (ARRAY[1,NULL]::int[] > ARRAY[1,2]) AS gt2,
  (ARRAY[1,NULL]::int[] < ARRAY[1,2]) AS lt2,
  (ARRAY[1,NULL]::int[] > ARRAY[1,0]) AS gt0,
  (ARRAY[1,NULL]::int[] < ARRAY[1,0]) AS lt0
;
