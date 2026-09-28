const boardPath=document.querySelector('.board-settings')?.dataset.boardPath||'';
const everyone=document.getElementById('board-all-access');
const accessControls=document.getElementById('board-access-controls');
const accessPeople=document.getElementById('board-access-people');
const accessCheckboxes=[...(accessPeople?.querySelectorAll('input[name="user_ids[]"]')||[])];
function updateAccessVisibility(){if(!everyone)return;accessControls.hidden=everyone.checked;accessPeople.classList.toggle('access-all',everyone.checked)}
everyone?.addEventListener('change',updateAccessVisibility);
updateAccessVisibility();
document.getElementById('board-access-filter')?.addEventListener('input',event=>{
  const term=event.target.value.toLocaleLowerCase();
  accessPeople.querySelectorAll('[data-board-access-person]').forEach(person=>{person.hidden=!person.textContent.toLocaleLowerCase().includes(term)});
});
document.getElementById('access-select-all')?.addEventListener('click',()=>{accessCheckboxes.forEach(input=>{input.checked=true})});
document.getElementById('access-select-none')?.addEventListener('click',()=>{accessCheckboxes.forEach(input=>{if(input.id!=='current-user-access')input.checked=false})});
async function submitBoardFrame(form){
  const response=await fetch(form.action,{method:'POST',headers:{Accept:'text/vnd.turbo-stream.html'},body:new URLSearchParams(new FormData(form))});
  if(!response.ok)return false;
  const streams=new DOMParser().parseFromString(await response.text(),'text/html').querySelectorAll('turbo-stream');
  streams.forEach(stream=>{
    const current=document.getElementById(stream.getAttribute('target'));
    const template=stream.querySelector('template');
    if(current&&template)current.replaceWith(template.content.cloneNode(true));
  });
  return true;
}
document.addEventListener('change',async event=>{
  if(event.target.matches('input[name="board[auto_postpone_period_in_days]"]')){
    const form=event.target.form;
    if(!await submitBoardFrame(form))alert('Could not change auto close period');
  }else if(event.target.matches('input[name="board[slider]"]')){
    const form=event.target.form;
    const radio=form.querySelectorAll('input[name="board[auto_postpone_period_in_days]"]')[Number(event.target.value)];
    if(radio){radio.checked=true;if(!await submitBoardFrame(form))alert('Could not change auto close period')}
  }else if(event.target.matches('#publication-toggle')){
    const previous=!event.target.checked;
    if(!await submitBoardFrame(event.target.form)){
      event.target.checked=previous;
      alert('Could not change public link');
    }
  }
});
const deleteDialog=document.getElementById('board-delete-dialog');
document.getElementById('delete-board')?.addEventListener('click',()=>deleteDialog?.showModal());
document.getElementById('cancel-delete-board')?.addEventListener('click',()=>deleteDialog?.close());
document.getElementById('settings-form')?.addEventListener('submit',event=>{
  const me=document.getElementById('current-user-access'),everyone=document.getElementById('board-all-access');
  if(me&&!me.checked&&!everyone.checked&&!confirm('Remove yourself from this board? You will need an invitation to regain access.'))event.preventDefault();
});
