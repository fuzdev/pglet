-- The well-formed one-dimensional forms: quoted and unquoted elements, both escapes, an
-- unquoted NULL in any case against a quoted or escaped one, the empty quoted element,
-- whitespace around elements and around the braces.
-- @test
SELECT '{a,"b,c",NULL,null,"NULL",\NULL,"","d\"e","f\\g",h\,i,j k}'::text[] AS a,
       ' { 1 , "2" ,NULL } '::int4[] AS b,
       '{ }'::int4[] AS c,
       '{" a ", b c }'::text[] AS d;
