const navButtons = document.querySelectorAll(".nav-btn");
const pages = document.querySelectorAll(".page");

function navigate(pageId) {
  navButtons.forEach((btn) => btn.classList.remove("active"));
  pages.forEach((page) => page.classList.remove("active"));

  document.querySelector(`[data-page="${pageId}"]`).classList.add("active");
  document.getElementById(`page-${pageId}`).classList.add("active");
}

navButtons.forEach((btn) => {
  btn.addEventListener("click", () => navigate(btn.dataset.page));
});
