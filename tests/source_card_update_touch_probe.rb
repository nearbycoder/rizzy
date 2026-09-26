# Run only against a disposable copy of the pinned Fizzy database.
require "json"

raise "Disposable Fizzy test database required" unless Rails.env.test? && ENV["RIZZY_DISPOSABLE_SOURCE_DB"] == "1"

account = Account.joins(:boards).first!
owner = account.users.find_by!(role: :owner)
board = account.boards.first!

def snapshot(card, board)
  card.reload
  board.reload
  { updated_at: card.updated_at, last_active_at: card.last_active_at,
    board_updated_at: board.updated_at, events: card.events.count }
end

def changes(before, after)
  {
    card_updated: after[:updated_at] > before[:updated_at],
    last_active_changed: after[:last_active_at] != before[:last_active_at],
    board_updated: after[:board_updated_at] > before[:board_updated_at],
    events_added: after[:events] - before[:events]
  }
end

result = nil
Current.set(account: account, user: owner) do
  card = Card.create!(account: account, board: board, creator: owner,
    status: "published", title: "Card update timestamp probe")
  before = snapshot(card, board)
  sleep 0.03
  card = Card.find(card.id)
  card.update!({})
  after_empty = snapshot(card, board)
  sleep 0.03
  card = Card.find(card.id)
  card.update!(description: "Description changed")
  after_description = snapshot(card, board)
  sleep 0.03
  card = Card.find(card.id)
  card.update!(title: "Card update timestamp probe renamed")
  after_title = snapshot(card, board)
  sleep 0.03
  card = Card.find(card.id)
  card.update!(title: "Card update timestamp probe renamed")
  after_same_title = snapshot(card, board)
  sleep 0.03
  card = Card.find(card.id)
  File.open(Rails.root.join("test/fixtures/files/moon.jpg"), "rb") do |file|
    card.update!(image: { io: file, filename: "moon.jpg", content_type: "image/jpeg" })
  end
  after_image = snapshot(card, board)
  result = {
    empty: changes(before, after_empty),
    description: changes(after_empty, after_description),
    title: changes(after_description, after_title),
    same_title: changes(after_title, after_same_title),
    image: changes(after_same_title, after_image)
  }
  card.image.purge
end

puts JSON.generate(result)
