-- A leading comment does not defeat this classifier, where it defeats its two
-- neighbours: `is_set_command`/`is_show_command` prefix-test the raw text, so
-- `/* trace */ SET …` is `0A000`, while this one reads the lexer, which has already
-- dropped comments. Query-tagging clients (sqlcommenter, ORM tracing) prefix every
-- statement, so the spelling is ordinary rather than exotic.
-- @test
/* trace */ DEALLOCATE ALL;
