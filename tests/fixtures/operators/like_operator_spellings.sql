-- The pattern-matching operators `LIKE`/`ILIKE` are transformed into on 18.6, at the
-- operator band rather than the predicate band, plus `^@` (`starts_with`).
-- @test
SELECT 'abc' ~~ 'a%' AS l, 'abc' !~~ 'b%' AS nl, 'ABC' ~~* 'a%' AS il,
       'ABC' !~~* 'b%' AS nil, 'abc' ^@ 'ab' AS sw;
