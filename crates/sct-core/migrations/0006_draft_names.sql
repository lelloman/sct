-- Match EntryName's Unicode character bound, as the mutable namespace does.
ALTER TABLE draft_entries DROP CONSTRAINT draft_entries_name_check;
ALTER TABLE draft_entries ADD CHECK(char_length(name) BETWEEN 1 AND 255 AND name NOT IN ('.','..') AND position('/' in name)=0);
