# Run only against a disposable copy of the pinned Fizzy database.
require "json"

raise "Disposable Fizzy test database required" unless Rails.env.test? && ENV["RIZZY_DISPOSABLE_SOURCE_DB"] == "1"

account = Account.joins(:boards).first!
owner = account.users.find_by!(role: :owner)
board = account.boards.first!
timestamp = Time.utc(2024, 1, 15, 10, 30)

result = nil
Current.set(account: account, user: owner) do
  ActiveRecord::Base.transaction do
    ids = 17.times.map do |index|
      Card.create!(account: account, board: board, creator: owner, status: "published",
        title: "Equal-time probe #{index}", created_at: timestamp).id
    end
    scope = Card.where(id: ids)
    oldest = scope.sorted_by("oldest").pluck(:id)
    newest = scope.sorted_by("newest").pluck(:id)
    raise "Oldest tie order differs" unless oldest == ids.sort
    raise "Newest tie order differs" unless newest == ids.sort.reverse
    raise "Creation times differ" unless scope.distinct.pluck(:created_at).one?
    result = { cards: ids.size, oldest_ascending_id: true, newest_descending_id: true }
    raise ActiveRecord::Rollback
  end
end

puts JSON.generate(result)
