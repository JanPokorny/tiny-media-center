import React from 'react'

function Footer({ leftText, rightText }) {
  return (
    <div className="footer">
      <div className="footer-left">{leftText}</div>
      <div className="footer-right">{rightText}</div>
    </div>
  )
}

export default Footer
