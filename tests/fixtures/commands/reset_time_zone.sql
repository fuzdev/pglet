-- `gram.y`'s `reset_rest` is not `RESET <one word>`: `TIME ZONE` is a fixed spelling
-- naming the GUC `timezone`. Reading one word made this `42601 syntax error at or
-- near "ZONE"` here where 18.6 answers `RESET`.
-- @test
RESET TIME ZONE;
