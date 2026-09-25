DROP TRIGGER IF EXISTS cards_search_insert;
DROP TRIGGER IF EXISTS cards_search_update;
DROP TRIGGER IF EXISTS cards_search_delete;
DROP TRIGGER IF EXISTS comments_search_insert;
DROP TRIGGER IF EXISTS comments_search_update;
DROP TRIGGER IF EXISTS comments_search_delete;

CREATE TRIGGER cards_search_insert AFTER INSERT ON cards WHEN NEW.status='published' BEGIN
  INSERT INTO card_search(rowid,title,description) VALUES(NEW.rowid,NEW.title,NEW.search_text);
END;
CREATE TRIGGER cards_search_update AFTER UPDATE OF title,search_text,status ON cards BEGIN
  DELETE FROM card_search WHERE rowid=OLD.rowid;
  INSERT INTO card_search(rowid,title,description) SELECT NEW.rowid,NEW.title,NEW.search_text WHERE NEW.status='published';
  DELETE FROM comment_search WHERE OLD.status<>NEW.status AND rowid IN (SELECT rowid FROM comments WHERE card_id=NEW.id);
  INSERT INTO comment_search(rowid,body) SELECT rowid,search_text FROM comments WHERE card_id=NEW.id AND OLD.status<>NEW.status AND NEW.status='published';
END;
CREATE TRIGGER cards_search_delete AFTER DELETE ON cards BEGIN
  DELETE FROM card_search WHERE rowid=OLD.rowid;
  DELETE FROM comment_search WHERE rowid IN (SELECT rowid FROM comments WHERE card_id=OLD.id);
END;
CREATE TRIGGER comments_search_insert AFTER INSERT ON comments WHEN EXISTS(SELECT 1 FROM cards WHERE id=NEW.card_id AND status='published') BEGIN
  INSERT INTO comment_search(rowid,body) VALUES(NEW.rowid,NEW.search_text);
END;
CREATE TRIGGER comments_search_update AFTER UPDATE OF search_text,card_id ON comments BEGIN
  DELETE FROM comment_search WHERE rowid=OLD.rowid;
  INSERT INTO comment_search(rowid,body) SELECT NEW.rowid,NEW.search_text WHERE EXISTS(SELECT 1 FROM cards WHERE id=NEW.card_id AND status='published');
END;
CREATE TRIGGER comments_search_delete AFTER DELETE ON comments BEGIN
  DELETE FROM comment_search WHERE rowid=OLD.rowid;
END;
